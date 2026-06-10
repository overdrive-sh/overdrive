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

## Alloc-status rendering — `render::alloc_status` is the LIVE path

`overdrive alloc status` renders through **`render::alloc_status(&AllocStatusOutput)`**
— and only that function. `main.rs` dispatches
`Command::Alloc(AllocCommand::Status { .. })` to `commands::alloc::status(..)`
and prints `render::alloc_status(&out)`. That is the one renderer an
operator ever sees.

`render.rs` contains **two other public renderers with confusingly
similar names that are NOT wired into any command** — they are
exercised *only by tests* (zero `src/` callers):

- `alloc_status_kind_aware(&AllocStatusResponse)` — branches on
  `WorkloadKind` (Service / Job / Schedule). **Test-only.**
- `alloc_snapshot(&AllocStatusResponse)` — the ADR-0033 §4 "journey
  TUI mockup". **Test-only.**

**A change made only to `alloc_status_kind_aware` or `alloc_snapshot`
does NOT reach an operator.** This has bitten more than one agent: you
grep for `alloc_status` + "render", land on the kind-aware function (it
looks the most complete — it has the `WorkloadKind` match arms), wire
your change there, write a green test against it — and the live
`overdrive alloc status` output is unchanged. The test passes; the
feature is broken. (Precedent: built-in-ca step 03-02 wired the
issued-certificates section into `alloc_status_kind_aware` only; it
shipped green and the operator saw nothing.)

Rules for any operator-visible change to `overdrive alloc status`
output:

1. **Make the change in `render::alloc_status`** (the live path). If you
   also keep it in `alloc_status_kind_aware` so the two do not drift —
   as the shared `render_vip_section` / `render_listeners_section` /
   `render_issued_certificates_section` helpers already are — that is
   fine, but `alloc_status` is the one that MUST change.
2. **Test the LIVE path.** Add or extend a test that calls
   `render::alloc_status(&AllocStatusOutput { snapshot:
   AllocStatusResponse { .. }, .. })` and asserts on its output. The
   live-path test home is `tests/acceptance/render_alloc_status.rs` —
   see the `(g)` listener-protocol test for the canonical shape and the
   comment there explaining that `main.rs` dispatches through
   `render::alloc_status`, NOT `alloc_status_kind_aware`. A test that
   only calls `alloc_status_kind_aware` / `alloc_snapshot` is testing
   the wrong surface and cannot defend operator-visible behaviour.
3. **Mind the signature difference.** `alloc_status` takes
   `&AllocStatusOutput`, so the response fields live on `out.snapshot.*`
   (e.g. `out.snapshot.issued_certificates`). The two test-only
   renderers take `&AllocStatusResponse` directly. Wiring a section into
   the live path means reading `out.snapshot.<field>`.

> The three-renderer split is a known hazard — but do NOT "fix" it by
> deleting or `#[cfg(test)]`-gating the unused pair on sight.
> `alloc_status_kind_aware` is the kind-aware renderer the
> **`workload-kind-discriminator` feature** built (commit `72175d7e`,
> step 02-02) to give `overdrive alloc status` Job-verdict /
> per-attempt-exit / Service-replica output. That feature is **done** —
> all its roadmap steps committed — but step 02-02 landed the renderer
> and a 442-line test suite while touching neither `main.rs` nor
> `commands/alloc.rs`: the wiring that makes the command dispatch through
> it was never done (its tests assert on `alloc_status_kind_aware`
> directly, so the step went green on the wrong surface). It has zero
> `src/` callers in all of git history, so the kind-aware operator view
> it built does not reach an operator.
> `alloc_snapshot` is an older ADR-0033 §4 TUI mockup. The real fix is to
> **finish that wiring** (dispatch the command through the kind-aware
> renderer and retire the flat `alloc_status`, folding the shared section
> helpers into the survivor), which is a `workload-kind-discriminator`
> completion decision — not a delete. Until that lands,
> `render::alloc_status` is the single source of operator-visible truth
> and the only renderer to change/test for operator-facing work.
