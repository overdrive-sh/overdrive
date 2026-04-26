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
- Handler arg structs (`SubmitArgs`, `StatusArgs`, `ListArgs`) carry
  `config_path: PathBuf` — no endpoint field.
- `SubmitOutput.endpoint` and the `CliError::Transport` rendering
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

The transport-error tests in `tests/integration/job_submit.rs` are
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
let output = overdrive_cli::commands::job::submit(
    SubmitArgs { spec: spec_path, config_path: cfg_path },
    &SimClock::new(),
    &SimTransport::new(),
).await?;
assert_eq!(output.outcome, IdempotencyOutcome::Inserted);
assert_eq!(output.spec_digest, expected_digest);
```

```rust
// Bad — subprocess; rejected
let out = Command::new(env!("CARGO_BIN_EXE_overdrive"))
    .args(["job", "submit", "payments.toml"])
    .output()?;
assert!(out.status.success());
```

### Exception

None in Phase 1. If a future test needs to verify `argv` parsing or
signal handling for the binary wrapper itself, it can exercise the
`main.rs` layer — but that is a test of `clap` configuration, not a
test of the CLI's behaviour, and it must be tagged and scoped
accordingly.
