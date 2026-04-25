# RCA — CLI ignores config endpoint

## Defect

`overdrive job submit ./payments.toml` fails with:

```
Error: could not reach the control plane at http://127.0.0.1:7001/.
Cause: transport error.
```

The operator config at `~/.overdrive/config` has `endpoint = "https://127.0.0.1:7001"`.
The server is listening on `https://127.0.0.1:7001`. The CLI is reaching for
`http://127.0.0.1:7001/` — wrong scheme — proving the config is never read.

## Root cause

The CLI's `--endpoint` flag has an unconditional clap `default_value = "http://127.0.0.1:7001"` at `crates/overdrive-cli/src/cli.rs:21`. Because `endpoint: String` has no "unset" state once a default fires, every handler passes `Some(args.endpoint.as_str())` to `ApiClient::from_config_with_endpoint`, which short-circuits the config-file fallback at `crates/overdrive-cli/src/http_client.rs:132`. The `unwrap_or_else(|| triple.endpoint())` fallback is unreachable in production.

The `http://` scheme in the error matches the clap default verbatim — smoking gun.

### Five Whys chain (condensed)

1. Error prints `http://127.0.0.1:7001/` → `ApiClient.base = http://…` at `http_client.rs:141`.
2. `self.base` is built from `endpoint_str` at `http_client.rs:132`, which resolves via `endpoint_override.unwrap_or_else(|| triple.endpoint())`.
3. Every handler passes `Some(...)`: `job.rs:105`, `cluster.rs:145`, `alloc.rs:93`, `node.rs:65`.
4. `args.endpoint` is always populated because clap has `default_value = "http://127.0.0.1:7001"` at `cli.rs:21`.
5. **Root**: the endpoint-precedence model was designed as flag → env → hardcoded default with no config fallback. The `Option<&str>` parameter in `from_config_with_endpoint` implies a "config fills in when None" shape — but clap makes `None` unreachable.

### Contributing factor

No integration test exercises endpoint resolution through the clap-to-handler seam. All existing tests pass `endpoint: Url` explicitly because they bind ephemeral ports. `from_config_with_endpoint(None, ...)` is only exercised once (`tests/integration/http_client.rs:67`) for CA pinning — never for endpoint resolution.

## User decision on fix direction

**Remove endpoint override entirely — config file is the sole source of truth.**

Rationale from the user: a flag/env override "defeats the purpose of the config."

Both `--endpoint` flag and `OVERDRIVE_ENDPOINT` env var are removed. No compat shim.

## Fix — single-cut

### Changes (greenfield, no grace period)

1. **`crates/overdrive-cli/src/cli.rs`** — remove the `endpoint` field from the root `Cli` struct entirely. Drop the clap `#[arg(long, env = ..., default_value = ...)]` attribute.

2. **`crates/overdrive-cli/src/main.rs`** — delete the `parse_cli_endpoint` helper. Remove every call site in `Command::JobSubmit`, `Command::ClusterStatus`, `Command::AllocStatus`, `Command::NodeList`. Handlers no longer receive an endpoint argument.

3. **`crates/overdrive-cli/src/commands/{job,cluster,alloc,node}.rs`** — drop `endpoint: Url` from each handler's args struct:
   - `SubmitArgs` (`commands/job.rs:39`)
   - `StatusArgs` (`commands/cluster.rs:112`)
   - `StatusArgs` (`commands/alloc.rs`)
   - `ListArgs` (`commands/node.rs`)

4. **`crates/overdrive-cli/src/http_client.rs`** — delete `ApiClient::from_config_with_endpoint` entirely. `ApiClient::from_config` becomes the only constructor used by handlers. Collapse `http_client.rs:132` to just `triple.endpoint()` (no `Option` parameter).

5. **Handler call sites** — replace every `ApiClient::from_config_with_endpoint(&args.config_path, Some(args.endpoint.as_str()))?` with `ApiClient::from_config(&args.config_path)?`:
   - `commands/job.rs:105`
   - `commands/cluster.rs:145`
   - `commands/alloc.rs:93`
   - `commands/node.rs:65`

6. **`crates/overdrive-cli/src/render.rs`** — `SubmitOutput.endpoint` renders from the resolved `client.base_url()` instead of `args.endpoint`. The `ApiClient` already exposes `base_url()` at `http_client.rs:147`. Plumb the resolved URL out of the handler into `SubmitOutput`.

### Files that will need mechanical test updates

- `crates/overdrive-cli/tests/integration/job_submit.rs` — drop `endpoint` from `SubmitArgs` construction.
- `crates/overdrive-cli/tests/integration/cluster_and_node_commands.rs` — drop `endpoint` from `StatusArgs` / `ListArgs` / `AllocStatusArgs` (lines ~61, 92, 117).
- `crates/overdrive-cli/tests/integration/cluster_init_serve.rs` — drop `endpoint` from args construction (~line 203).
- `crates/overdrive-cli/tests/integration/http_client.rs` — drop any test paths that only existed to cover the `endpoint_override` parameter. If `ApiClient::from_config_with_endpoint` disappears, any test exercising it either migrates to `from_config` or is deleted (the test of a deleted function is also deleted).

### Test strategy — the RED scaffold

**New acceptance test**: `crates/overdrive-cli/tests/integration/endpoint_from_config.rs`.

Shape:
1. Bind a TLS server on an ephemeral port (reuse the `TestServer` harness from `cluster_init_serve.rs` / `job_submit.rs`).
2. Write the operator config file pointing `endpoint` at the server's resolved `https://127.0.0.1:<ephemeral>` URL.
3. Call the `job submit` handler with args that *do not* include `endpoint` (because the field is now removed).
4. Assert the server received the POST — proves the client read the endpoint from the config.

This test **fails with the current code** because `SubmitArgs` still has `endpoint: Url` and the handler still passes `Some(...)` to `from_config_with_endpoint`. It **passes with the fix** because `from_config` reads `triple.endpoint()` as the only source.

Wire the module into `crates/overdrive-cli/tests/integration.rs`. Gate it behind `integration-tests` if the crate already gates its integration lane there; otherwise leave it in the default lane.

### Why this is RED-then-GREEN, not refactor

The bug is a behavioural defect: the operator's documented workflow (`cluster init` writes config → subsequent commands read it) is broken. The regression test reproduces the exact user-visible failure mode. The fix restores the documented workflow. Removing the `endpoint` field is the minimal change that makes the regression test pass.

### Risk

- Confined to `crates/overdrive-cli`. No cross-crate fanout.
- No wire-format or schema change. Config file on-disk shape unchanged.
- Compile-breaking by design: Rust enforces an exhaustive update of every call site. No silent miss possible.
- No regression possible: the config-read path has never worked, so no operator can be depending on the broken behaviour. Anyone relying on `--endpoint` / `OVERDRIVE_ENDPOINT` is explicitly told to use the config instead — this is the intended operator model per the whitepaper (§8 *Operator Identity and CLI Authentication*: "the resulting CA trust bundle, active operator SVID, and context … live at `~/.overdrive/config`").

## Repo conventions to honour

- **Single-cut**: no deprecation path, no `#[deprecated]`, no grace period. Delete old + land new in the same commit.
- **Newtypes**: `ApiClient::base_url()` returns the canonical form — use it, don't re-parse.
- **Test runner**: `cargo nextest run`, not `cargo test`. Doctests via `cargo test --doc` if touched.
- **No `.feature` files**: pure Rust `#[tokio::test]`.
- **No env-mutating tests needed**: since `OVERDRIVE_ENDPOINT` is being removed, no test depends on the env var, and `#[serial(env)]` is not required.
- **No CI/lefthook wiring changes requested**: the new test is in the existing integration lane and will be picked up automatically.

## Commit shape

Single commit, conventional message:

```
fix(cli): read endpoint from config, remove --endpoint/OVERDRIVE_ENDPOINT

The clap default on --endpoint made the flag always-set, which short-circuited
the config-file fallback in ApiClient::from_config_with_endpoint. The config
was never consulted. Remove the override surface entirely: the operator config
at ~/.overdrive/config is now the sole source of the client endpoint, matching
the documented workflow (cluster init writes; subsequent commands read).

Regression test: tests/integration/endpoint_from_config.rs — bind server on
ephemeral port, write config pointing at it, invoke job submit without
endpoint args, assert POST reaches the server.
```
