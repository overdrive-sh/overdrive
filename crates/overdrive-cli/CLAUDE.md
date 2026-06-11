# overdrive-cli conventions

## Endpoint resolution — config only

The operator config at `~/.overdrive/config` is the sole source of the
client endpoint. The CLI reads the endpoint, CA pin, and operator SVID
from one trust triple; the SVID, CA pin, and endpoint are a single
unit per whitepaper §8, and partial override would break the unit.
Same reasoning as ADR-0010 §R4 on `--insecure`: the trust posture is
not runtime-tunable per command.

### Mechanics

- `Cli` has no `endpoint` field — only `command`.
- `ApiClient::from_config(config_path)` is the only constructor.
- Handler arg structs (`DeployArgs`, `StatusArgs`, `ListArgs`) carry
  `config_path: PathBuf` — no endpoint field.
- `DeployOutput.endpoint` and the `CliError::Transport` rendering
  both read from `ApiClient::base_url()` — the endpoint the trust
  triple names is the only source.
- `overdrive serve` writes the trust triple *after* binding the
  listener, so the recorded endpoint names the resolved port (not
  the requested bind — which may be `:0` under tests and dev flows).

### Tests

Tests do NOT pass an endpoint to handlers. They start a server on an
ephemeral port and invoke the handler with just `config_path`; the
trust triple `overdrive serve` writes already names the live
endpoint. See `tests/integration/endpoint_from_config.rs` for the
canonical shape.

The transport-error tests in `tests/integration/deploy.rs` are
the only exception — they overwrite the config's endpoint with an
unreachable one (`127.0.0.1:1`) to exercise `CliError::Transport`.

### Operator flows that need a different endpoint

Swap the active config via `$OVERDRIVE_CONFIG_DIR`. That moves the
*whole* trust triple, not just the endpoint. The unit of trust is the
config file; it is never partially overridden.

## Integration tests — no subprocess

**Do not spawn `overdrive` as a subprocess in tests.** Call the CLI
command handlers directly as Rust functions. This is a firm rule, not a
default — we have rejected the "invoke the binary via `Command::spawn`"
pattern for this crate.

### Why

- **Deterministic.** Subprocess tests depend on shell quoting, stdout
  buffering, environment inheritance, and process-start timing. Direct
  calls have none of those. DST discipline (§21 whitepaper) applies to
  the CLI's logic just as it applies to reconcilers — injected `Clock`,
  `Transport`, `Entropy` through the same trait surface.
- **Fast.** A test suite that spawns `overdrive` 100× pays 100× a
  fork/exec. Calling handler functions is in-process.
- **Composable fakes.** The same `SimTransport` / `SimClock` that drive
  the rest of the DST harness drive CLI tests too — there is no second
  fixture style.
- **Honest failure signals.** A subprocess failure gives you an exit
  code and stderr; a direct call gives you a typed `Result<_, Error>`
  the test can branch on variant-by-variant.

### What this requires of the crate structure

The CLI exposes a **library surface** (`src/lib.rs`) alongside the thin
binary entry point (`src/main.rs`). The binary's job is only:

1. Install `color-eyre` + `tracing-subscriber`.
2. Parse `argv` via `clap`.
3. Construct the real production adapters (`SystemClock`,
   `TcpTransport`, reqwest `Client` against `~/.overdrive/config`).
4. Call into `overdrive_cli::run(command, adapters).await` and map
   `eyre::Report` to the process exit code.

Everything else — command handlers, output rendering, config loading,
error-to-message mapping — lives in the library. Tests import the
library and call handlers directly with `SimTransport` / `SimClock` /
in-memory config.

### What to write in `tests/acceptance/*.rs`

```rust
// Good — direct call, injected sim adapters, typed assertion
let output = overdrive_cli::commands::deploy::deploy(
    DeployArgs { spec: spec_path, config_path: cfg_path },
    &SimClock::new(),
    &SimTransport::new(),
).await?;
assert_eq!(output.outcome, IdempotencyOutcome::Inserted);
assert_eq!(output.spec_digest, expected_digest);
```

```rust
// Bad — subprocess; rejected
let out = Command::new(env!("CARGO_BIN_EXE_overdrive"))
    .args(["deploy", "payments.toml"])
    .output()?;
assert!(out.status.success());
```

### Exception

None in Phase 1. If a future test needs to verify `argv` parsing or
signal handling for the binary wrapper itself, it can exercise the
`main.rs` layer — but that is a test of `clap` configuration, not a
test of the CLI's behaviour, and it must be tagged and scoped
accordingly.

## Alloc-status rendering — ONE renderer: `render::alloc_status`

`overdrive alloc status` renders through **`render::alloc_status(&AllocStatusOutput)`**
— and only that function. `main.rs` dispatches
`Command::Alloc(AllocCommand::Status { .. })` to `commands::alloc::status(..)`
and prints `render::alloc_status(&out)`. That is the one renderer an
operator ever sees, and it is now the ONLY alloc-status renderer in
`render.rs`. There is no test-only duplicate to mistake it for.

`render::alloc_status` carries the **kind-aware** body that the
`workload-kind-discriminator` feature designed (step 02-02, [D4] /
ADR-0047 §4): it branches on `out.snapshot.kind`
(`overdrive_core::aggregate::WorkloadKind`) via the private
`render_kind_aware_body` helper —

- **Service**: `Service '<name>' (kind: Service)` header + `Spec
  digest:` + `Replicas (desired/running): N/M` + per-alloc table
  (`Alloc / State / Restarts / Since`, NO Exit column; a Failed alloc's
  cause renders on an indented detail line beneath its row, preserving
  RCA S-A4).
- **Job**: `Job '<name>' (kind: Job)` header + `Spec digest:` +
  `Verdict:` (derived) + per-attempt table (`Attempt / State / Exit /
  Started / Duration`) + stderr tail on Failed.
- **Schedule**: `Schedule '<name>' (kind: Schedule)` header + cron
  tracking URL.

— plus, after the kind body, the shared presence-guarded
`render_vip_section` / `render_listeners_section` /
`render_issued_certificates_section`, and an empty-state onboarding
signpost (`phase-1-first-workload`, DWD-05) rendered first when
`out.allocations_total == 0`.

### History — why this was a trap, and how it was closed

`render.rs` used to carry **three** alloc-status renderers, and the
wrong one was live:

- `alloc_status` was a FLAT renderer (`Workload ID:` / `Spec digest:` /
  `Allocations:` + bare per-row lines) — it was live but did NOT branch
  on kind.
- `alloc_status_kind_aware(&AllocStatusResponse)` had the designed
  kind-aware body but had **zero `src/` callers** — the
  `workload-kind-discriminator` step 02-02 built it (commit `72175d7e`)
  and a 442-line test suite, but never wired `main.rs` /
  `commands/alloc.rs` through it (its tests asserted on
  `alloc_status_kind_aware` directly, so the step went green on the
  wrong surface and the kind-aware operator view never shipped).
- `alloc_snapshot(&AllocStatusResponse)` was an older ADR-0033 §4
  "journey TUI mockup", also test-only with zero `src/` callers.

The consolidation (this branch) **finished the wiring**: the kind-aware
body folded into the single live `render::alloc_status` (threading
`AllocStatusOutput` so the empty-state signpost survives), the flat
duplicate was retired single-cut, and `alloc_snapshot` + its tests were
deleted (its transition-history mockup behavior — `Restart budget:` /
`Last transition:` arrows / `source: driver(exec)` — had no live caller
and is no longer produced). The authoritative tests moved onto the live
`render::*` path.

### Rules for any operator-visible change to `overdrive alloc status`

1. **Make the change in `render::alloc_status`** (or the private
   `render_kind_aware_body` / shared section helpers it calls). There is
   no second renderer to keep in sync.
2. **Test the LIVE path.** The live-path test home is
   `tests/acceptance/render_alloc_status.rs` — call
   `render::alloc_status(&AllocStatusOutput { snapshot:
   AllocStatusResponse { .. }, .. })` and assert on its output. See the
   `(j)` kind-aware Job/Service tests and the `(g)` / `(g2)` / `(h)` /
   `(i)` section tests for the canonical shapes. The render-layer
   integration suite (`tests/integration/alloc_status.rs`) exercises the
   same live renderer via its `render_live(..)` wrapper.
3. **Mind the field source.** `alloc_status` takes `&AllocStatusOutput`,
   so the snapshot fields live on `out.snapshot.*` (e.g.
   `out.snapshot.issued_certificates`, `out.snapshot.kind`,
   `out.snapshot.spec_digest`). The wrapper-level `out.workload_id` /
   `out.spec_digest` / `out.allocations_total` / `out.empty_state_message`
   are the command-derived envelope fields; the kind-aware body reads the
   server-populated snapshot fields (which the live `commands::alloc::status`
   populates from the `GET /v1/allocs` response).
