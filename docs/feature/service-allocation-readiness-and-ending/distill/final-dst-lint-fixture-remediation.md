# DISTILL fixture remediation — async filesystem I/O

## Scope and cause

At source baseline `f13990f434515d848d99954cd40ec80ce6c07d06`, the canonical Metal `cargo xtask dst-lint` run `ae3dac63ba6e4596d52154e9` reported five blocking `std::fs` calls inside the source-local async shared-board acceptance fixture (`guest_listener_board_wiring_acceptance.rs:126,133,134,195,196`). The retained output is `/private/tmp/service-readiness-final-dst-lint.log`. `.claude/rules/rust.md` § “Concurrency & async” requires awaited `tokio::fs` at this boundary.

The fixture now awaits `tokio::fs::create_dir_all` for the staging, data and operator-config directories and `tokio::fs::write` for the stand-in kernel and rootfs. Its private `read_ca` helper also awaits `tokio::fs::read_to_string`; this removes a synchronous filesystem read on the same async boot path. Fixture paths, creation order, failure expectations, Service submission, beacon inputs, exact startup `Pass` oracle, and per-test Contract Shape declaration are unchanged. No production code, public API, test identity, or test universe changed.

## Verification and remaining diagnostic

| Command or evidence | Result |
|---|---|
| `rustfmt --edition 2024 --check crates/overdrive-control-plane/src/guest_listener_board_wiring_acceptance.rs`; `git diff --check` | Passed |
| `cargo xtask metal run -- cargo xtask dst-lint` | Run `2efb5fe2c8c0b64453015293`: the five fixture diagnostics are gone. Exit 1 remained for a separate feature-caused scanner mismatch at `crates/overdrive-control-plane/src/lib.rs:1`: the new 05-02 production call was `compose_and_probe_runner_gate_with_reports`, while the scanner recognized only `compose_and_probe_runner_gate`. That diagnostic was also present before this fixture edit; `lib.rs` was unchanged here. The scanner was corrected in `b23de5ca7`, independently audited in `9b387cb26`; the later full Metal DST lint run `dbf2700f5458905e4584797e` passed. My isolated fixture run itself did not pass the whole lint. |
| `cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`; `cargo xtask metal run -- cargo xtask guest-kit-prepare` | Passed in isolated Metal checkout, runs `cd575835dafa4a75de4790ce` and `57d9406bcaff4af3923ec287`; these prepared the existing fixture lane. |
| `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib -E 'test(a_listen_line_on_the_drivers_beacon_completes_the_probe_tasks_declared_set)'` | Run `17727855873b17020af8ca15`: 1/1 passed after lane preparation. An earlier selected attempt returned `lane_not_prepared` before the test body; it was setup evidence, not an assertion failure. |

The separate scanner mismatch was resolved by its owner. No mutation testing or DES phase event was run for this fixture remediation.
