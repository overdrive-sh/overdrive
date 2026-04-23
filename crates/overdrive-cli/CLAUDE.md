# overdrive-cli conventions

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
    SubmitArgs { spec: spec_path, endpoint: sim_endpoint },
    &SimClock::new(),
    &SimTransport::new(),
).await?;
assert_eq!(output.commit_index, 1);
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
