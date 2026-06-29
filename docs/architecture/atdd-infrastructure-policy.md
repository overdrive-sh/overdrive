# ATDD Infrastructure Policy

Per `nw-distill` § Project Infrastructure Policy. One file per project. Apply-if-exists; write-if-absent; rewrite with `--policy=fresh`. Git history is the audit trail.

> **Polyglot note (Rust workspace).** This project is Rust, governed by
> `.claude/rules/testing.md` (four-tier model) and `.claude/rules/development.md`.
> The generic DISTILL skill's Python machinery (`pytest-bdd` `.feature` files,
> `steps_*.py`, the `nwave_ai.state_delta` Python port at
> `tests/common/state_delta.py`) is **NOT** used here and MUST NOT be
> introduced — `.claude/rules/testing.md` § "No `.feature` files anywhere"
> overrides it. Acceptance tests are Rust `#[test]`/`#[tokio::test]`
> functions. Gherkin GIVEN/WHEN/THEN blocks live as specification-only prose
> in `docs/feature/{id}/distill/test-scenarios.md`.
>
> **Mandate 8 (Universe-bound state-delta) mapping.** The `assert_state_delta`
> Python port has no Rust equivalent in this workspace. The equivalent
> universe-bound discipline is satisfied natively: for the dataplane
> `update_service` surface, the **universe** is the port-observable
> `BTreeSet<BackendKey>` REVERSE_NAT key set (plus the forward-path service
> map), and the **expected** assertion is exact set-equality against the keys
> derived from `(frontend.proto, backends)`. The `ReverseNatLockstep`
> invariant's `check_lockstep` (forward-direction *every expected key present*
> + reverse-direction *no orphan key*) IS the fail-closed universe guard —
> an unexpected extra key (e.g. the phantom `udp` key for a TCP-only service)
> fails the orphan check exactly as a `strict=True` state-delta would.

## Driving
| Port | Mechanism | Note |
|---|---|---|
| `overdrive deploy <spec.toml>` (CLI) | real subprocess of the built `overdrive` binary, real control-plane chain, inside Lima (`cargo xtask lima run --`) | Walking skeleton (US-04). Asserts exit code + `Accepted.` stdout + the UDP reverse wire path. CLI verb is `Deploy` (`cli.rs:42`, `main.rs:63`) — NOT `job submit`. |
| `ServiceMapHydrator.reconcile` (in-process driving port) | direct call, pure sync (ADR-0035) | Emits `Action::DataplaneUpdateService` (+ proto). Used by the C3-guard Tier-1 scenarios. |
| action-shim `dispatch` (Action → `update_service`) | direct call; builds `ServiceFrontend::new` (the IPv6-reject site) | The driving adapter into the `Dataplane` port. Used by the D1a IPv6-rejection Tier-1 scenario. |
| `overdrive workload restart <id>` (CLI) — *backend-instance-replacement #249* | real subprocess of the built `overdrive` binary inside Lima (`cargo xtask lima run --`) → `POST /v1/jobs/:id/restart` | New `workload` namespace (ADR-0073, mechanism pinned — not an open choice). S-BIR-CLI-RESTART-SUCCESS (exit 0 + stdout) / -UNKNOWN (non-zero + `not_found`). |
| `restart_workload` (in-process HTTP handler) — *#249* | direct in-process axum handler call (mirrors `stop_workload`); test-double `IntentStore` (`CountingIntentStore` / `FaultInjectingIntentStore`) for focused coverage | S-BIR-HANDLER-404 / -TXN / -OUTCOME-RESUMED / -OUTCOME-RESTARTED. In-process (`@driving_adapter`), NOT `@real-io` — the real-I/O proof of the same path is the CLI subprocess row above. |
| `WorkloadLifecycle.reconcile` (in-process driving port) — *#249* | direct call, pure sync (ADR-0035); constructed `(desired, actual, view, tick)` | The generation-gated placement + current-instance-scoped veto. S-BIR-RESTART-*, S-BIR-STOP-ONCE, S-BIR-COALESCE-PLACE, S-BIR-COALESCE-NO-REPLAY, S-BIR-SEQUENTIAL, S-BIR-REGRESSION-*, S-BIR-BUG3-PRESERVED, S-BIR-CURRENT-ALLOC. |

## Driven internal (real)
| Port | Mechanism | Note |
|---|---|---|
| `Dataplane` (`update_service`) — Sim adapter | `SimDataplane` in-process (`BTreeMap`/`BTreeSet`), real DST | Tier 1 (`cargo dst`). `adapter-sim` class; this is the lockstep set-equality home. The Sim is the "real" driven-internal adapter for the in-process tier — it honours the same `Dataplane` trait contract. |
| `Dataplane` (`update_service`) — Ebpf adapter | `EbpfDataplane` real aya-rs + bpffs + real veth, inside Lima | Tier 3 (`cargo xtask lima run`, `integration-tests` feature). `bpftool map dump REVERSE_NAT_MAP` + AF_PACKET/tcpdump wire capture. The production-adapter half of the cross-adapter equality. |
| `xdp_reverse_nat_lookup` (kernel program) | `BPF_PROG_TEST_RUN` triptych (PKTGEN/SETUP/CHECK) | Tier 2 (`cargo xtask bpf-unit`). Real kernel verifier + program execution against a synthetic proto=17 packet. |
| `IntentStore` (`txn` w/ NEW `TxnOp::IncrementU64` + `Delete`; `get`/`delete`) — Local adapter — *#249* | `LocalIntentStore` over real `redb` (`TempDir`), Tier-1 store-acceptance gated `integration-tests` | S-BIR-TXN-01..04 (`@real-io`: N concurrent ⇒ final == N, atomic monotonic). The same `get`/`delete` path is also exercised through the production route at Tier-3 by S-BIR-CLI-RESTART-SUCCESS/-UNKNOWN. The `current_alloc` helper + the BE-u64 codec are pure (no port trait) — Tier-1 unit/proptest seams, not adapters. |

## Driven external / non-deterministic (fake)
| Port | Fake | Note |
|---|---|---|
| (none new) | — | This feature introduces **no new external/non-deterministic driven port** (no clock, email, SMS, payment, LLM, third-party API). The only non-determinism is real-kernel wire timing in Tier 3, handled by the existing `overdrive-testing` netns/veth fixtures (`ThreeIfaceTopology`) + a wall-clock capture budget, not a fake. Per DESIGN [REF] Driven ports: "no new external dependency is introduced." |
| (none new) — *backend-instance-replacement #249* | — | The restart feature introduces **no new external/non-deterministic driven port** either. The reconciler reads wall-clock only via the existing injected `tick.now`; the in-flight-churn surface is the **reused** Tier-3 intercept worker `TCP_USER_TIMEOUT`/keepalive legs (real, NOT `sock_destroy` — #61 scope), not a fake. |
