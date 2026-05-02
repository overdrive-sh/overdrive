# Walking skeleton — wire-exec-spec-end-to-end

Designer: Quinn. Date: 2026-04-30. Strategy decision recorded as
DWD-2 in `wave-decisions.md`.

> **Amended 2026-04-30 per ADR-0031 Amendment 1.** The driving port
> (`overdrive_cli::commands::job::submit(...)` called as a Rust
> function), the strategy (Strategy C — real local resources), the
> resource matrix, and the data-flow diagram are all UNCHANGED. What
> changes is the back-door IntentStore read assertion shape: the
> rkyv-archived `Job` now carries `driver: WorkloadDriver` (tagged
> enum) instead of flat `command` / `args` fields. The assertion
> destructures `let WorkloadDriver::Exec(exec) = &job.driver;` and
> reads `exec.command` / `exec.args`. The TOML input the operator
> writes is unchanged (still `[exec]` table with `command` and
> `args` keys).

## Strategy: C — Real local resources, no container

The single end-to-end happy-path scenario for this feature
(`@walking_skeleton @driving_port`) calls
`overdrive_cli::commands::job::submit(...)` directly as a Rust
function (per `crates/overdrive-cli/CLAUDE.md` § *Integration tests
— no subprocess*) against an in-process `serve::run` server bound
to `127.0.0.1:0`. The data flow exercised end-to-end:

```
TempDir/payments.toml               (real on-disk TOML)
        │ toml::from_str
        ▼
JobSpecInput                         (client-side parse)
        │ Job::from_spec
        ▼
Job aggregate                        (client-side validation, CM-A entry port)
        │ POST /v1/jobs (real reqwest over rustls localhost)
        ▼
handlers::submit_job                 (server-side handler — defence-in-depth Job::from_spec)
        │ rkyv::to_bytes
        ▼
LocalIntentStore (real redb under TempDir)
        │ broker enqueue
        ▼
JobLifecycle::reconcile              (pure projection — Action::StartAllocation { spec })
        │ action_shim::dispatch_single
        ▼
SimDriver::start                     (boundary substitute — see "What InMemory cannot model")
```

## Driving port

`overdrive_cli::commands::job::submit(SubmitArgs { spec, config_path })`
called as a Rust async function. This IS the project-canonical
"driving adapter verification" form — the
`crates/overdrive-cli/CLAUDE.md` rule explicitly overrides the
nWave skill's `subprocess` mandate.

The same WS exercises the server-side `handlers::submit_job` driving
port indirectly: the CLI's reqwest client speaks real HTTP over real
rustls to the in-process axum server. No mock HTTP, no in-memory
Service trait — the `127.0.0.1:0` port assignment is dynamic, the
trust triple is on disk, the cert is rcgen-minted at boot per the
existing `phase-1-control-plane-core` walking-skeleton pattern.

## Resources used

| Surface | Real adapter | Notes |
|---|---|---|
| TOML on disk | `tempfile::TempDir` + `std::fs::write` | The operator's authoritative input is a real filesystem read. |
| `IntentStore` | `LocalIntentStore::open(TempDir/intent.redb)` | Real redb file, real ACID writes. |
| `ObservationStore` | `SimObservationStore::single_peer` | In-memory CRDT — see "What InMemory cannot model" below. |
| `Driver` | `SimDriver::new(DriverType::Exec)` | NOT real `tokio::process::Command::new`. |
| `Clock` | `SimClock::new()` | DST-controllable; the WS does not depend on wall-clock progress for its happy path. |
| HTTP transport | real `reqwest::Client` over rustls + `127.0.0.1:0` axum-server | Real TLS handshake, real wire bytes. |
| Trust triple | rcgen-minted CA + leaf in TempDir | Same pattern as existing `phase-1-control-plane-core` WS. |

## What InMemory cannot model — and why it's fine here

Two adapters substitute for production at the WS boundary:

### `SimObservationStore` instead of real Corrosion

The Phase 1 production `ObservationStore` is `LocalObservationStore`
(per ADR-0012 revision 2026-04-24). A real `CorrosionStore` (with
SWIM gossip + CR-SQLite LWW) is Phase 2+ scope. The DESIGN-side
state-layer hygiene check (Architecture brief.md § State-layer
hygiene) is the type system; it does not need a live Corrosion peer
to validate the rkyv-archive shape of a `Job` carrying `command` +
`args`. **Adapter integration coverage** for `LocalObservationStore`
is exercised by `crates/overdrive-store-local/tests/integration/`
under `--features integration-tests` — the WS does not duplicate
that.

### `SimDriver` instead of real `ExecDriver` (`tokio::process`)

The `ExecDriver` integration — actually spawning a real subprocess
via `tokio::process::Command::new(&spec.command).args(&spec.args)`,
attaching it to a cgroup, detecting OOM, etc. — is exercised by the
existing `crates/overdrive-control-plane/tests/integration/job_lifecycle/submit_to_running.rs`
suite gated behind `--features integration-tests`. That suite IS the
adapter integration coverage for the `ExecDriver`. The WS exercises
the *intent surface* (CLI → handler → constructor → store → action
projection), where the value the operator declared (`/opt/x/y --port
8080`) appears intact at every hop.

Substituting `SimDriver` at the WS boundary is appropriate per the
skill's "Strategy C — Real local resources" guidance: the WS proves
end-to-end wiring at the data-flow boundary that matters for *this*
feature (the spec carries through), not at the boundary where this
feature did NOT change anything (the driver consumes the spec).

The WS-litmus question — "if I deleted the real `LocalIntentStore`
adapter and substituted an InMemory one, would this WS still pass?"
— is "no" for this WS: the rkyv archive byte-equality assertion
against the redb-stored bytes (via `state.store.get(b"jobs/payments")`
— the same back-door read the existing `submit_job_idempotency.rs`
acceptance test uses) requires the real LocalIntentStore. That is
the load-bearing real adapter for this feature.

## What the WS asserts

1. `submit(...)` returns `Ok(SubmitOutput { outcome: Inserted, .. })`
   — the wire-level "fresh insert" witness per ADR-0020.
2. `SubmitOutput.spec_digest` is byte-identical to a locally-computed
   `ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed)))` —
   proves the rkyv lane is consistent across client and server.
3. `SubmitOutput.intent_key == "jobs/payments"` — proves
   `IntentKey::for_job` derivation flows through.
4. The IntentStore at key `jobs/payments` carries an rkyv-archived
   `Job` whose `driver` matches `WorkloadDriver::Exec(Exec { command,
   args })` with `command == "/opt/payments/bin/payments-server"`
   and `args == vec!["--port", "8080"]` — the load-bearing assertion
   that pins this feature's value-delivery. Per ADR-0031 Amendment 1
   the `Job` aggregate now carries a tagged-enum `driver:
   WorkloadDriver` field; the test destructures the enum to reach
   the inner exec-shape values. (See DWD-20 in `wave-decisions.md`.)

## What the WS does NOT assert

- That the driver actually spawns the binary (covered by
  `submit_to_running.rs` integration).
- That OpenAPI YAML on disk is regenerated (covered by the dedicated
  `openapi_exec_block.rs` acceptance scenario in §7 of
  `test-scenarios.md` and the existing `xtask openapi-check` CI gate).
- That the convergence loop runs to completion against a real
  `ExecDriver` (covered by existing
  `runtime_convergence_loop.rs` after the fixture migration).

These are deliberately split off into focused scenarios so the WS
remains a 1-scenario gate Ana would recognise as "I submitted a job
and the platform agreed it was valid and stored what I said."
