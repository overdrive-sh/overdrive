# DISTILL review F-01 remediation — D10 handler Contract Shape

The independent `review-final-oracle-alignment.md` iteration 1 found that the transitioned `s_shcp_wire_13_cap_timer_synthesises_service_failed_timeout` test lacked its own Contract Shape declaration. The test drives the production Service submit handler and advances `SimClock` to observe the bounded Timeout wire outcome, so its shape is `bounded-change`.

The existing test now carries `/// CONTRACT_SHAPE: bounded-change.` immediately above its attributes. A function-local `clippy::doc_markdown` allowance names this required machine-read marker, following the narrow allowance already used by the CLI Timeout render test. The handler test body, derived deadline, exact typed Timeout assertion, seed, and test identity are unchanged.

| Check | Result |
|---|---|
| `rustfmt --edition 2024 --check crates/overdrive-control-plane/tests/acceptance/service_submit_dispatch_wiring.rs`; `git diff --check` | Passed. |
| `cargo xtask metal run -- cargo clippy -p overdrive-control-plane --test acceptance --features integration-tests -- -D warnings` | Passed, Metal run `fff4f70185f4ae8881e6966b`. The isolated run's BPF prerequisite was prepared successfully by run `b9c341304216ded2de41bac8`. |
| Prior focused D10 runtime evidence | The same test body passed in selected Metal run `b482a5e4f8d02acc81b5a8d6`; this declaration-only remediation did not rerun native tests. |

No production code, public API, model, scenario body, DES phase event, or mutation configuration changed. The user's mutation waiver remains in effect. This bounded correction returns F-01 to the same reviewer for iteration 2.
