# RED classification — udp-service-support (DISTILL → DELIVER gate)

**Date:** 2026-06-02. **Author:** Sentinel (acceptance-designer).

Per `nw-distill` § "Pre-DELIVER fail-for-the-right-reason gate". DELIVER reads
this at RED-phase entry to confirm RED is genuine (implementation missing), not
BROKEN (import/fixture/setup error).

## Gate mechanism (project convention)

Per `.claude/rules/testing.md` § "RED scaffolds", the RED-ready signal in this
Rust workspace is `#[should_panic(expected = "RED scaffold")]` **passing at the
bar** — the scaffold is GREEN-at-the-runner (panics as expected) so sibling
commits never need `--no-verify`, while remaining structurally tied to the
unimplemented spec. This replaces the generic skill's
`AssertionError`-vs-`ImportError` Python classification.

## Classification

| Scenario | Scaffold | Classification | Why |
|---|---|---|---|
| S-01-A / S-01-B | `overdrive-core/tests/service_frontend.rs` | RED (MISSING_FUNCTIONALITY) | drives `ServiceFrontend::new`/`vip_v4` production `todo!("RED scaffold: …")` — panics with "RED scaffold"; imports resolve (`ServiceFrontend` re-exported at `dataplane/mod.rs:32`, verified) |
| S-01-C / D / E | `overdrive-core/tests/service_frontend_provenance.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!("… RED scaffold (S-01-…)")`; no production import beyond core (compiles) |
| S-01-F | `overdrive-control-plane/tests/acceptance/service_frontend_ipv6_rejected.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!`; wired into `acceptance.rs` entrypoint |
| S-02-A..D, S-03-A..D | `overdrive-sim/tests/sim_dataplane_reverse_nat_per_proto.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!`; standalone entrypoint (no extra wiring) |
| S-03-E / F | `overdrive-bpf/tests/integration/xdp_reverse_nat_udp.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!`; wired into `integration.rs`; gated `integration-tests` |
| S-04-A..C | `overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!`; wired into `integration.rs`; gated `integration-tests` |
| S-05-A..C | `overdrive-dataplane/tests/integration/multi_listener_tcp_udp_e2e.rs` | RED (MISSING_FUNCTIONALITY) | test-side `panic!`; wired into `integration.rs`; gated `integration-tests` |
| S-02-E | (none — confirmation against shipped `Proto::try_from`) | n/a | boundary already shipped by #164; DELIVER adds a thin assertion, no scaffold |

## Pre-flight verification performed

- `cargo xtask lima run -- cargo check -p overdrive-core` (library): **PASS** —
  the production `ServiceFrontend` `todo!` stub compiles clean; the
  `#[expect(clippy::todo, …)]` gates the scaffold bodies.
- Import-path audit: `ServiceFrontend` (`dataplane/mod.rs:32`), `Proto`
  (`backend_key.rs:66`), `ServiceVip` (`id.rs:650`, `new()` confirmed) all
  resolve. No `ImportError`/`ModuleNotFoundError` shape.

## Known environment caveat (NOT a scaffold defect)

`cargo check -p overdrive-core --tests` and any `--tests` check touching
`overdrive-dataplane` (a transitive dev-dep via `overdrive-sim`) requires
`target/bpf/overdrive_bpf.o` (`overdrive-dataplane/build.rs` hard-fails without
it — `feedback_bpf_object_prereq_for_trybuild.md`). In this Conductor+Lima
workspace the `cargo xtask bpf-build` step is currently blocked by a host/guest
`target/` incremental-compilation lock (`Permission denied (os error 13)`),
unrelated to the scaffolds. DELIVER (or CI on Linux) runs `cargo xtask
bpf-build` first; the test scaffolds then compile-and-RED. The library check
above is the authoritative scaffold-compile signal that does not depend on the
BPF object.
